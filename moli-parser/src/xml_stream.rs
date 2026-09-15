use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

use html5ever::{
    Attribute as HtmlAttribute, LocalName as HtmlLocalName, Namespace as HtmlNamespace,
    Prefix as HtmlPrefix, QualName as HtmlQualName,
    tendril::StrTendril as HtmlStrTendril,
    tree_builder::{ElementFlags as HtmlElementFlags, NodeOrText as HtmlNodeOrText},
};
use moli_dom::native::{DomHost, NativeDom, NativeNodeId};
use url::Url;
use xml5ever::{
    Attribute as XmlAttribute, ExpandedName, QualName as XmlQualName, TokenizerResult,
    buffer_queue::BufferQueue,
    driver::XmlParseOpts,
    interface::{ElementFlags as XmlElementFlags, QuirksMode as XmlQuirksMode},
    tendril::StrTendril,
    tokenizer::{ProcessResult, Token, TokenSink, XmlTokenizer},
    tree_builder::{NodeOrText as XmlNodeOrText, TreeSink, XmlTreeBuilder},
};

use crate::{
    ParserBlockingStylesheetPause, ParserDomMutationConsumer, ParserDomReadConsumer,
    ParserElementCreationConsumer, ParserFinishDiscoverySignals, ParserMutationEffectConsumer,
    ParserPlanningReadView, ParserPumpOutcome, ParserPumpStep, ParserScriptHandoff, ParserYield,
    PreparedScript,
    html::ParseHandle,
    live_target::{ParserRuntimeDomSinks, ParserStreamHtmlTreeSinkTarget},
    stream::prepare_parser_script_handoff_for_static_document,
    xml_tree_viewer::transform_parser_target_to_xml_tree_view,
};

/// Incremental XML backend for an executable Document.
///
/// Unlike xml5ever's high-level `XmlParser` driver, this owner surfaces
/// `TokenizerResult::Script` instead of immediately feeding through it. The
/// tokenizer, tree builder, unconsumed input, and live-DOM target therefore
/// remain in one parser session across script and stylesheet suspension.
pub struct XmlDocumentStream {
    parser: XmlParserSession,
    input: XmlParserInputStream,
    eof_declared: bool,
}

#[derive(Default)]
struct XmlParserInputStream {
    end_segments: VecDeque<String>,
}

struct XmlParserSession {
    tokenizer: XmlTokenizer<EmbedderPausingXmlTreeBuilder>,
    input_buffer: BufferQueue,
}

struct EmbedderPausingXmlTreeBuilder {
    inner: XmlTreeBuilder<XmlStreamParseHandle, XmlStreamDocumentSink>,
}

struct XmlStreamDocumentSink {
    target: RefCell<XmlParserStreamTarget>,
    quirks_mode: Cell<XmlQuirksMode>,
}

struct XmlParserStreamTarget {
    common: ParserStreamHtmlTreeSinkTarget,
    present_unstyled_top_level_document: bool,
}

#[derive(Debug, Clone)]
struct XmlStreamParseHandle {
    inner: ParseHandle,
    element_name: Option<Rc<XmlQualName>>,
    suppress_append: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawXmlParserStep {
    Script(NativeNodeId),
    BlockingStylesheet(NativeNodeId),
    InputDrained,
}

impl XmlDocumentStream {
    /// Starts a raw XML stream. This is suitable for parser tests and inert
    /// consumers; executable top-level navigation uses
    /// `new_top_level_document` explicitly.
    pub fn new(final_url: Url) -> Self {
        Self::from_target(ParserStreamHtmlTreeSinkTarget::new_xml(final_url), false)
    }

    pub fn new_top_level_document(final_url: Url) -> Self {
        Self::from_target(ParserStreamHtmlTreeSinkTarget::new_xml(final_url), true)
    }

    pub fn new_live_document_root(final_url: Url, document_handle: NativeNodeId) -> Self {
        Self::from_target(
            ParserStreamHtmlTreeSinkTarget::new_live_xml_document_root(final_url, document_handle),
            false,
        )
    }

    fn from_target(
        common: ParserStreamHtmlTreeSinkTarget,
        present_unstyled_top_level_document: bool,
    ) -> Self {
        let target = XmlParserStreamTarget {
            common,
            present_unstyled_top_level_document,
        };
        Self {
            parser: XmlParserSession::new(target, XmlParseOpts::default()),
            input: XmlParserInputStream::default(),
            eof_declared: false,
        }
    }

    /// Appends decoded XML bytes to the parser-owned end segment chain.
    /// Appending does not run the tokenizer or mutate the DOM.
    pub fn append_to_end(&mut self, chunk: String) {
        assert!(!self.eof_declared, "cannot append XML input after EOF");
        if chunk.is_empty() {
            return;
        }
        self.input.end_segments.push_back(chunk);
    }

    /// Declares source EOF. Calling this more than once is a no-op.
    pub fn declare_eof(&mut self) {
        if self.eof_declared {
            return;
        }
        self.eof_declared = true;
    }

    pub fn has_pending_input(&self) -> bool {
        self.parser.has_buffered_input() || !self.input.end_segments.is_empty()
    }

    pub fn next_input_len(&self) -> usize {
        if self.parser.has_buffered_input() {
            self.parser.buffered_input_len()
        } else {
            self.input
                .end_segments
                .front()
                .map(String::len)
                .unwrap_or_default()
        }
    }

    pub fn snapshot_pending_input(&self) -> String {
        let mut pending = self.parser.snapshot_buffered_input();
        for segment in &self.input.end_segments {
            pending.push_str(segment);
        }
        pending
    }

    pub fn pump_next_parser_step(&mut self, max_bytes: usize) -> ParserPumpOutcome {
        let chunk = self.take_next_owned_input(max_bytes);
        self.parser.pump_parser_step(&chunk)
    }

    pub fn pump_next_parser_step_with_runtime_dom_consumer<T>(
        &mut self,
        max_bytes: usize,
        consumer: &mut T,
    ) -> ParserPumpOutcome
    where
        T: ParserDomReadConsumer
            + ParserDomMutationConsumer
            + ParserMutationEffectConsumer
            + ParserElementCreationConsumer,
    {
        // SAFETY: `consumer` remains exclusively borrowed until the scoped
        // guard clears every erased callback at the end of this parser step.
        let sinks = unsafe { ParserRuntimeDomSinks::from_consumer(consumer) };
        self.parser.enter_runtime_dom_sinks_parse_step(sinks);
        let step = RuntimeXmlDomSinksParserStep { stream: self };
        step.stream.pump_next_parser_step(max_bytes)
    }

    pub fn snapshot_parser_stream_document(&self) -> NativeDom {
        self.parser
            .sink()
            .borrow()
            .common
            .snapshot_parser_stream_document()
    }

    pub fn snapshot_parser_stream_dom_host(&self) -> DomHost {
        self.parser
            .sink()
            .borrow()
            .common
            .snapshot_parser_stream_dom_host()
    }

    pub fn take_parser_stream_dom_host(&mut self) -> DomHost {
        self.parser
            .sink()
            .borrow_mut()
            .common
            .take_parser_stream_document()
    }

    pub fn restore_parser_stream_dom_host(&mut self, dom_host: DomHost) {
        self.parser
            .sink()
            .borrow_mut()
            .common
            .restore_parser_stream_dom_host(dom_host);
    }

    pub fn set_document_content_type(&mut self, content_type: String) {
        let mut dom_host = self.take_parser_stream_dom_host();
        let document_handle = dom_host.document_handle();
        let _ = dom_host.set_document_content_type_for_handle(document_handle, content_type);
        self.restore_parser_stream_dom_host(dom_host);
    }

    pub fn with_parser_stream_dom_host_for_bootstrap<R>(
        &mut self,
        f: impl FnOnce(DomHost) -> std::result::Result<R, Box<(anyhow::Error, DomHost)>>,
    ) -> anyhow::Result<R> {
        let bootstrap_document = self.take_parser_stream_dom_host();
        match f(bootstrap_document) {
            Ok(result) => Ok(result),
            Err(error) => {
                let (error, bootstrap_document) = *error;
                self.restore_parser_stream_dom_host(bootstrap_document);
                Err(error)
            }
        }
    }

    pub fn take_parser_stream_null_custom_element_registry_elements(
        &mut self,
    ) -> Vec<NativeNodeId> {
        self.parser
            .sink()
            .borrow_mut()
            .common
            .take_parser_stream_null_custom_element_registry_elements()
    }

    pub fn with_stylesheet_blocking_read_view<R>(
        &self,
        f: impl FnOnce(&dyn moli_stylesheet_blocking::StylesheetBlockingReadView) -> R,
    ) -> R {
        let target = self.parser.sink().borrow();
        f(&target.common)
    }

    pub fn drain_discovered_parser_meta_csp_candidates(&mut self) -> Vec<NativeNodeId> {
        self.parser
            .sink()
            .borrow_mut()
            .common
            .drain_discovered_parser_meta_csp_candidates()
    }

    pub fn mark_script_already_started(&mut self, node_id: NativeNodeId) {
        self.parser
            .sink()
            .borrow_mut()
            .common
            .mark_script_already_started(node_id);
    }

    pub fn finish_with_runtime_dom_consumer<T>(
        mut self,
        consumer: &mut T,
    ) -> ParserFinishDiscoverySignals
    where
        T: ParserDomReadConsumer
            + ParserDomMutationConsumer
            + ParserMutationEffectConsumer
            + ParserElementCreationConsumer,
    {
        self.declare_eof();
        assert!(
            !self.has_pending_input(),
            "XML parser must drain all source before finish"
        );
        // SAFETY: `consumer` remains exclusively borrowed for the consuming
        // finish operation. The unwind guard clears callbacks if finish panics.
        let sinks = unsafe { ParserRuntimeDomSinks::from_consumer(consumer) };
        self.parser.enter_runtime_dom_sinks_parse_step(sinks);
        let mut finish = RuntimeXmlDomSinksParserFinish {
            parser: Some(self.parser),
        };
        finish.finish()
    }

    pub fn finish(mut self) -> NativeDom {
        self.declare_eof();
        while self.has_pending_input() {
            let _ = self.pump_next_parser_step(0);
        }
        self.parser.finish_owned().finish_dom_host().into_dom()
    }

    fn take_next_owned_input(&mut self, max_bytes: usize) -> String {
        if self.parser.has_buffered_input() {
            return String::new();
        }
        let Some(input) = self.input.end_segments.pop_front() else {
            return String::new();
        };
        let (prefix, remainder) = split_xml_parser_input_prefix(input, max_bytes);
        if let Some(remainder) = remainder {
            self.input.end_segments.push_front(remainder);
        }
        prefix
    }
}

impl XmlParserSession {
    fn new(target: XmlParserStreamTarget, opts: XmlParseOpts) -> Self {
        let sink = XmlStreamDocumentSink {
            target: RefCell::new(target),
            quirks_mode: Cell::new(XmlQuirksMode::NoQuirks),
        };
        let tree_builder = XmlTreeBuilder::new(sink, opts.tree_builder);
        let tokenizer = XmlTokenizer::new(
            EmbedderPausingXmlTreeBuilder {
                inner: tree_builder,
            },
            opts.tokenizer,
        );
        Self {
            tokenizer,
            input_buffer: BufferQueue::default(),
        }
    }

    fn sink(&self) -> &RefCell<XmlParserStreamTarget> {
        &self.tokenizer.sink.inner.sink.target
    }

    fn has_buffered_input(&self) -> bool {
        !self.input_buffer.is_empty()
    }

    fn buffered_input_len(&self) -> usize {
        let input = self.input_buffer.clone();
        let mut len = 0usize;
        while let Some(chunk) = input.pop_front() {
            len = len.saturating_add(chunk.len());
        }
        len
    }

    fn snapshot_buffered_input(&self) -> String {
        let input = self.input_buffer.clone();
        let mut buffered = String::new();
        while let Some(chunk) = input.pop_front() {
            buffered.push_str(&chunk);
        }
        buffered
    }

    fn enter_runtime_dom_sinks_parse_step(&mut self, sinks: ParserRuntimeDomSinks) {
        self.sink()
            .borrow_mut()
            .common
            .enter_runtime_dom_sinks_parse_step(sinks);
    }

    fn clear_runtime_dom_sinks_after_parse_step(&mut self) {
        self.sink()
            .borrow_mut()
            .common
            .clear_runtime_dom_sinks_after_parse_step();
    }

    fn pump_parser_step(&mut self, chunk: &str) -> ParserPumpOutcome {
        if !chunk.is_empty() {
            self.input_buffer.push_back(StrTendril::from(chunk));
        }

        let raw_step = loop {
            match self.tokenizer.feed(&self.input_buffer) {
                TokenizerResult::Done => break RawXmlParserStep::InputDrained,
                TokenizerResult::EncodingIndicator(_) => continue,
                TokenizerResult::Script(handle) => {
                    let node_id = handle.inner.node_id();
                    let target = self.sink().borrow();
                    if target.common.front_pending_blocking_stylesheet_pause() == Some(node_id) {
                        break RawXmlParserStep::BlockingStylesheet(node_id);
                    }
                    if ParserPlanningReadView::parser_script_read(&target.common, node_id).is_some()
                    {
                        break RawXmlParserStep::Script(node_id);
                    }
                    // xml5ever yields on every local-name `script`, including
                    // non-HTML namespaces. Those nodes are not executable and
                    // therefore do not suspend an executable Document parser.
                }
            }
        };

        let mut target = self.sink().borrow_mut();
        let async_candidate_ids = target.common.drain_discovered_async_prefetch_candidates();
        let discovered_async_prefetch_scripts = async_candidate_ids
            .iter()
            .filter_map(|node_id| {
                match prepare_parser_script_handoff_for_static_document(
                    &target.common,
                    *node_id,
                    0,
                    0,
                ) {
                    ParserScriptHandoff::AsyncPostParse { script, .. } => Some(script),
                    _ => None,
                }
            })
            .collect::<Vec<PreparedScript>>();
        let discovered_modulepreload_link_candidates = target
            .common
            .drain_discovered_modulepreload_link_candidates();
        let discovered_blocking_stylesheet_inputs =
            target.common.drain_discovered_blocking_stylesheet_inputs();

        let result = match raw_step {
            RawXmlParserStep::Script(node_id) => {
                let handoff = prepare_parser_script_handoff_for_static_document(
                    &target.common,
                    node_id,
                    0,
                    0,
                );
                ParserPumpStep::Yield(ParserYield::Script(Box::new(handoff)))
            }
            RawXmlParserStep::BlockingStylesheet(node_id) => {
                let pending = target.common.pop_pending_blocking_stylesheet_pause();
                assert_eq!(pending, Some(node_id));
                ParserPumpStep::Yield(ParserYield::BlockingStylesheet(
                    ParserBlockingStylesheetPause { node_id },
                ))
            }
            RawXmlParserStep::InputDrained => ParserPumpStep::InputDrained,
        };

        ParserPumpOutcome {
            result,
            discovered_async_prefetch_scripts,
            discovered_modulepreload_link_candidates,
            discovered_blocking_stylesheet_inputs,
        }
    }

    fn finish_owned(self) -> ParserStreamHtmlTreeSinkTarget {
        let Self {
            tokenizer,
            input_buffer,
        } = self;
        debug_assert!(input_buffer.is_empty());
        tokenizer.end();
        let mut target = tokenizer.sink.inner.sink.finish();
        target.present_unstyled_top_level_document_if_needed();
        target.common
    }

    fn finish_live_runtime_dom_sink_parser(self) -> ParserFinishDiscoverySignals {
        let Self {
            tokenizer,
            input_buffer,
        } = self;
        debug_assert!(input_buffer.is_empty());
        tokenizer.end();
        let mut target = tokenizer.sink.inner.sink.target.borrow_mut();
        target.present_unstyled_top_level_document_if_needed();
        ParserFinishDiscoverySignals {
            parser_created_null_registry_elements: target
                .common
                .take_parser_stream_null_custom_element_registry_elements(),
            discovered_modulepreload_link_candidates: target
                .common
                .drain_discovered_modulepreload_link_candidates(),
            discovered_parser_meta_csp_candidates: target
                .common
                .drain_discovered_parser_meta_csp_candidates(),
            discovered_blocking_stylesheet_inputs: target
                .common
                .drain_discovered_blocking_stylesheet_inputs(),
        }
    }
}

impl TokenSink for EmbedderPausingXmlTreeBuilder {
    type Handle = XmlStreamParseHandle;

    fn process_token(&self, token: Token) -> ProcessResult<Self::Handle> {
        let result = self.inner.process_token(token);
        if !matches!(result, ProcessResult::Continue) {
            return result;
        }
        let pending_stylesheet = self
            .inner
            .sink
            .target
            .borrow()
            .common
            .front_pending_blocking_stylesheet_pause();
        if let Some(node_id) = pending_stylesheet {
            return ProcessResult::Script(XmlStreamParseHandle::new(ParseHandle::new(
                node_id, None,
            )));
        }
        ProcessResult::Continue
    }

    fn end(&self) {
        self.inner.end();
    }
}

impl XmlParserStreamTarget {
    fn present_unstyled_top_level_document_if_needed(&mut self) {
        if self.present_unstyled_top_level_document {
            transform_parser_target_to_xml_tree_view(&mut self.common);
        }
    }
}

impl XmlStreamParseHandle {
    fn new(inner: ParseHandle) -> Self {
        Self {
            inner,
            element_name: None,
            suppress_append: false,
        }
    }

    fn element(inner: ParseHandle, element_name: Rc<XmlQualName>) -> Self {
        Self {
            inner,
            element_name: Some(element_name),
            suppress_append: false,
        }
    }

    fn suppressed(inner: ParseHandle) -> Self {
        Self {
            inner,
            element_name: None,
            suppress_append: true,
        }
    }
}

impl PartialEq for XmlStreamParseHandle {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for XmlStreamParseHandle {}

impl TreeSink for XmlStreamDocumentSink {
    type Handle = XmlStreamParseHandle;
    type Output = XmlParserStreamTarget;
    type ElemName<'a>
        = ExpandedName<'a>
    where
        Self: 'a;

    fn finish(self) -> Self::Output {
        self.target.into_inner()
    }

    fn parse_error(&self, error: Cow<'static, str>) {
        self.target
            .borrow_mut()
            .common
            .push_parse_error(error.into_owned());
    }

    fn get_document(&self) -> Self::Handle {
        XmlStreamParseHandle::new(self.target.borrow().common.document_handle())
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> Self::ElemName<'a> {
        target
            .element_name
            .as_deref()
            .expect("xml5ever requested the name of a non-element node")
            .expanded()
    }

    fn create_element(
        &self,
        name: XmlQualName,
        attrs: Vec<XmlAttribute>,
        flags: XmlElementFlags,
    ) -> Self::Handle {
        let mut target = self.target.borrow_mut();
        let element_name = Rc::new(name.clone());
        let html_name = xml_name_to_html(&name);
        let html_attrs = attrs.into_iter().map(xml_attribute_to_html).collect();
        let mut html_flags = HtmlElementFlags::default();
        html_flags.template = flags.template;
        html_flags.mathml_annotation_xml_integration_point =
            flags.mathml_annotation_xml_integration_point;
        XmlStreamParseHandle::element(
            target
                .common
                .create_element(html_name, html_attrs, html_flags, None),
            element_name,
        )
    }

    fn create_comment(&self, text: StrTendril) -> Self::Handle {
        XmlStreamParseHandle::new(
            self.target
                .borrow_mut()
                .common
                .create_comment(text.to_string()),
        )
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Self::Handle {
        let target_text = target.to_string();
        let data_text = data.to_string();
        let mut parser_target = self.target.borrow_mut();
        let handle = parser_target
            .common
            .create_processing_instruction(target_text.clone(), data_text);
        if target_text.eq_ignore_ascii_case("xml") {
            XmlStreamParseHandle::suppressed(handle)
        } else {
            XmlStreamParseHandle::new(handle)
        }
    }

    fn append(&self, parent: &Self::Handle, child: XmlNodeOrText<Self::Handle>) {
        let child = match child {
            XmlNodeOrText::AppendNode(handle) if handle.suppress_append => return,
            XmlNodeOrText::AppendNode(handle) => HtmlNodeOrText::AppendNode(handle.inner),
            XmlNodeOrText::AppendText(text) => {
                HtmlNodeOrText::AppendText(HtmlStrTendril::from(text.as_ref()))
            }
        };
        let mut target = self.target.borrow_mut();
        let parent_id = target
            .common
            .template_contents_parent_handle(parent.inner.node_id())
            .map(|handle| handle.node_id())
            .unwrap_or_else(|| parent.inner.node_id());
        target.common.append(parent_id, child).consume();
    }

    fn append_before_sibling(&self, sibling: &Self::Handle, child: XmlNodeOrText<Self::Handle>) {
        let child = match child {
            XmlNodeOrText::AppendNode(handle) if handle.suppress_append => return,
            XmlNodeOrText::AppendNode(handle) => HtmlNodeOrText::AppendNode(handle.inner),
            XmlNodeOrText::AppendText(text) => {
                HtmlNodeOrText::AppendText(HtmlStrTendril::from(text.as_ref()))
            }
        };
        self.target
            .borrow_mut()
            .common
            .append_before_sibling(sibling.inner.node_id(), child)
            .consume();
    }

    fn append_based_on_parent_node(
        &self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: XmlNodeOrText<Self::Handle>,
    ) {
        let child = match child {
            XmlNodeOrText::AppendNode(handle) if handle.suppress_append => return,
            XmlNodeOrText::AppendNode(handle) => HtmlNodeOrText::AppendNode(handle.inner),
            XmlNodeOrText::AppendText(text) => {
                HtmlNodeOrText::AppendText(HtmlStrTendril::from(text.as_ref()))
            }
        };
        self.target
            .borrow_mut()
            .common
            .append_based_on_parent_node(
                element.inner.node_id(),
                prev_element.inner.node_id(),
                child,
            )
            .consume();
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        self.target
            .borrow_mut()
            .common
            .append_doctype(
                name.to_string(),
                public_id.to_string(),
                system_id.to_string(),
            )
            .consume();
    }

    fn mark_script_already_started(&self, node: &Self::Handle) {
        self.target
            .borrow_mut()
            .common
            .mark_script_already_started(node.inner.node_id());
    }

    fn pop(&self, node: &Self::Handle) {
        let is_script = node.inner.element_name.as_ref().is_some_and(|name| {
            name.local.as_ref() == "script"
                && matches!(
                    name.ns.as_ref(),
                    "http://www.w3.org/1999/xhtml" | "http://www.w3.org/2000/svg"
                )
        });
        self.target
            .borrow_mut()
            .common
            .note_node_closed(node.inner.node_id(), is_script);
    }

    fn get_template_contents(&self, target: &Self::Handle) -> Self::Handle {
        self.target
            .borrow_mut()
            .common
            .template_contents_handle(target.inner.node_id())
            .map(XmlStreamParseHandle::new)
            .unwrap_or_else(|| target.clone())
    }

    fn same_node(&self, left: &Self::Handle, right: &Self::Handle) -> bool {
        left == right
    }

    fn set_quirks_mode(&self, mode: XmlQuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn add_attrs_if_missing(&self, target: &Self::Handle, attrs: Vec<XmlAttribute>) {
        self.target.borrow_mut().common.add_attrs_if_missing(
            target.inner.node_id(),
            attrs.into_iter().map(xml_attribute_to_html).collect(),
        );
    }

    fn remove_from_parent(&self, target: &Self::Handle) {
        self.target
            .borrow_mut()
            .common
            .remove_from_parent(target.inner.node_id())
            .consume();
    }

    fn reparent_children(&self, node: &Self::Handle, new_parent: &Self::Handle) {
        self.target
            .borrow_mut()
            .common
            .reparent_children(node.inner.node_id(), new_parent.inner.node_id())
            .consume();
    }
}

impl xml5ever::tree_builder::XmlTreeSink for XmlStreamDocumentSink {
    fn create_cdata(&self, text: StrTendril) -> Self::Handle {
        XmlStreamParseHandle::new(
            self.target
                .borrow_mut()
                .common
                .create_cdata_section(text.to_string()),
        )
    }
}

fn xml_name_to_html(name: &XmlQualName) -> HtmlQualName {
    HtmlQualName::new(
        name.prefix
            .as_ref()
            .map(|prefix| HtmlPrefix::from(prefix.as_ref())),
        HtmlNamespace::from(name.ns.as_ref()),
        HtmlLocalName::from(name.local.as_ref()),
    )
}

fn xml_attribute_to_html(attribute: XmlAttribute) -> HtmlAttribute {
    HtmlAttribute {
        name: xml_name_to_html(&attribute.name),
        value: HtmlStrTendril::from(attribute.value.as_ref()),
    }
}

fn split_xml_parser_input_prefix(input: String, max_bytes: usize) -> (String, Option<String>) {
    if max_bytes == 0 || input.len() <= max_bytes {
        return (input, None);
    }
    let mut split = input.len();
    for (index, character) in input.char_indices() {
        let end = index + character.len_utf8();
        if end > max_bytes {
            split = if index == 0 { end } else { index };
            break;
        }
    }
    let remainder = input[split..].to_owned();
    let prefix = input[..split].to_owned();
    (prefix, (!remainder.is_empty()).then_some(remainder))
}

struct RuntimeXmlDomSinksParserStep<'a> {
    stream: &'a mut XmlDocumentStream,
}

impl Drop for RuntimeXmlDomSinksParserStep<'_> {
    fn drop(&mut self) {
        self.stream
            .parser
            .clear_runtime_dom_sinks_after_parse_step();
    }
}

struct RuntimeXmlDomSinksParserFinish {
    parser: Option<XmlParserSession>,
}

impl RuntimeXmlDomSinksParserFinish {
    fn finish(&mut self) -> ParserFinishDiscoverySignals {
        self.parser
            .take()
            .map(XmlParserSession::finish_live_runtime_dom_sink_parser)
            .unwrap_or_default()
    }
}

impl Drop for RuntimeXmlDomSinksParserFinish {
    fn drop(&mut self) {
        if let Some(parser) = &mut self.parser {
            parser.clear_runtime_dom_sinks_after_parse_step();
        }
    }
}

#[cfg(test)]
mod tests {
    use moli_dom::native::{Node, NodeType};
    use url::Url;

    use crate::{ParserPumpStep, ParserYield, XmlDocumentStream};

    fn test_url() -> Url {
        Url::parse("https://example.test/page.xhtml").expect("test URL")
    }

    fn element_by_id(
        document: &moli_dom::native::NativeDom,
        id: &str,
    ) -> Option<moli_dom::native::NativeNodeId> {
        document
            .nodes()
            .find(|node| {
                node.as_element()
                    .and_then(|element| element.attribute("id"))
                    == Some(id)
            })
            .map(Node::id)
    }

    #[test]
    fn incremental_xml_stops_before_future_dom_at_parser_script() {
        let mut stream = XmlDocumentStream::new(test_url());
        stream.append_to_end(
            concat!(
                "<html xmlns='http://www.w3.org/1999/xhtml'><body>",
                "<div id='before'/><script>globalThis.hit = true;</script>",
                "<div id='future'/></body></html>"
            )
            .to_owned(),
        );
        stream.declare_eof();

        let first = stream.pump_next_parser_step(0);
        let ParserPumpStep::Yield(ParserYield::Script(handoff)) = first.result else {
            panic!("expected an executable XML parser script handoff");
        };
        let crate::ParserScriptHandoff::BlockingClassic { script, .. } = *handoff else {
            panic!("inline XHTML script must use the parser-blocking classic lane");
        };
        assert!(matches!(
            script.source,
            crate::ScriptSource::Inline(ref source)
                if source == "globalThis.hit = true;"
        ));
        let paused = stream.snapshot_parser_stream_dom_host();
        assert!(paused.element_handle_by_id("before").is_some());
        assert!(
            paused.element_handle_by_id("future").is_none(),
            "XML future nodes must not be materialized past a parser script"
        );

        let second = stream.pump_next_parser_step(0);
        assert!(matches!(second.result, ParserPumpStep::InputDrained));
        let resumed = stream.snapshot_parser_stream_dom_host();
        assert!(resumed.element_handle_by_id("future").is_some());
    }

    #[test]
    fn incremental_xml_preserves_chunked_cdata_and_namespace_declarations() {
        let mut stream = XmlDocumentStream::new(test_url());
        for chunk in [
            "<root data-before='1' xml",
            "ns:m='urn:meta'><child><![CDA",
            "TA[ < > & ]]></child></root>",
        ] {
            stream.append_to_end(chunk.to_owned());
            while stream.has_pending_input() {
                let outcome = stream.pump_next_parser_step(3);
                assert!(matches!(outcome.result, ParserPumpStep::InputDrained));
            }
        }
        stream.declare_eof();
        while stream.has_pending_input() {
            let outcome = stream.pump_next_parser_step(3);
            assert!(matches!(outcome.result, ParserPumpStep::InputDrained));
        }

        let document = stream.finish();
        let root = document
            .document_element_node_id()
            .expect("document element");
        let attributes = document
            .node(root)
            .and_then(Node::as_element)
            .expect("root element")
            .attributes();
        assert_eq!(
            attributes
                .iter()
                .map(|attribute| attribute.name())
                .collect::<Vec<_>>(),
            ["data-before", "xmlns:m"]
        );
        let child = document
            .child_ids(root)
            .find(|node_id| document.node(*node_id).is_some_and(Node::is_element))
            .expect("child element");
        let cdata = document.child_ids(child).next().expect("CDATA child");
        assert_eq!(
            document.node(cdata).map(Node::node_type),
            Some(NodeType::CDataSection)
        );
        assert_eq!(
            document
                .node(cdata)
                .and_then(Node::as_cdata_section)
                .map(|section| section.data()),
            Some(" < > & ")
        );
    }

    #[test]
    fn incremental_xhtml_appends_template_children_to_template_contents() {
        let mut stream = XmlDocumentStream::new(test_url());
        stream.append_to_end(
            concat!(
                "<html xmlns='http://www.w3.org/1999/xhtml'><body>",
                "<template id='outer'>text<div id='outer-child'/>",
                "<template id='inner'><span id='inner-child'/></template></template>",
                "<p id='after'/></body></html>"
            )
            .to_owned(),
        );
        let document = stream.finish();
        let outer = element_by_id(&document, "outer").expect("outer template");
        let inner = element_by_id(&document, "inner").expect("inner template");
        let outer_child = element_by_id(&document, "outer-child").expect("outer child");
        let inner_child = element_by_id(&document, "inner-child").expect("inner child");
        let after = element_by_id(&document, "after").expect("sibling after template");
        let outer_contents = document
            .node(outer)
            .and_then(Node::as_element)
            .and_then(moli_dom::native::Element::template_contents)
            .expect("outer template contents");
        let inner_contents = document
            .node(inner)
            .and_then(Node::as_element)
            .and_then(moli_dom::native::Element::template_contents)
            .expect("inner template contents");
        let contents_document = document
            .node(outer_contents)
            .and_then(Node::owner_document)
            .expect("template contents owner document");

        assert_eq!(document.child_ids(outer).count(), 0);
        assert_eq!(document.child_ids(inner).count(), 0);
        assert_eq!(
            document.node(inner).and_then(Node::parent_node),
            Some(outer_contents)
        );
        assert_eq!(
            document.node(outer_child).and_then(Node::owner_document),
            Some(contents_document)
        );
        assert_eq!(
            document.node(inner_child).and_then(Node::owner_document),
            Some(contents_document)
        );
        assert_eq!(
            document.node(inner_contents).and_then(Node::owner_document),
            Some(contents_document)
        );
        assert_eq!(
            document.node(after).and_then(Node::owner_document),
            Some(document.document_node_id())
        );
    }

    #[test]
    fn incremental_xml_preserves_namespaces_across_many_single_byte_chunks() {
        let mut source = String::from("<root>");
        for index in 0..512 {
            source.push_str(&format!(
                "<p:item xmlns:p='urn:item:{index}' data-index='{index}'/>"
            ));
        }
        source.push_str("</root>");

        let mut stream = XmlDocumentStream::new(test_url());
        for byte in source.bytes() {
            stream.append_to_end(char::from(byte).to_string());
        }
        let document = stream.finish();
        let items = document
            .nodes()
            .filter_map(Node::as_element)
            .filter(|element| element.local_name() == "item")
            .collect::<Vec<_>>();
        assert_eq!(items.len(), 512);
        for (index, item) in items.into_iter().enumerate() {
            assert_eq!(item.namespace(), format!("urn:item:{index}"));
            let attributes = item.attributes();
            assert_eq!(
                attributes
                    .iter()
                    .map(|attribute| attribute.name())
                    .collect::<Vec<_>>(),
                ["xmlns:p", "data-index"]
            );
            assert_eq!(attributes[0].namespace(), "http://www.w3.org/2000/xmlns/");
            assert_eq!(attributes[0].value(), format!("urn:item:{index}"));
        }
    }

    #[test]
    fn incremental_xml_keeps_cdata_syntax_in_comment_and_pi() {
        let mut stream = XmlDocumentStream::new(test_url());
        for chunk in ["<root><!-- <![CDA", "TA[not closed -->"] {
            stream.append_to_end(chunk.to_owned());
            while stream.has_pending_input() {
                let outcome = stream.pump_next_parser_step(2);
                assert!(matches!(outcome.result, ParserPumpStep::InputDrained));
            }
        }

        let snapshot = stream.snapshot_parser_stream_document();
        assert_eq!(
            snapshot
                .nodes()
                .find_map(Node::as_comment)
                .map(|comment| comment.data()),
            Some(" <![CDATA[not closed "),
            "a fake CDATA opener inside a closed comment must not retain later input"
        );

        for chunk in [
            "<?moli-cdata 0?><?probe <![CDATA[pi]]>?>",
            "<child><![CDA",
            "TA[real]]></child></root>",
        ] {
            stream.append_to_end(chunk.to_owned());
        }
        let document = stream.finish();
        let instructions = document
            .nodes()
            .filter_map(Node::as_processing_instruction)
            .map(|instruction| (instruction.target(), instruction.data()))
            .collect::<Vec<_>>();
        assert_eq!(
            instructions,
            [("moli-cdata", "0"), ("probe", "<![CDATA[pi]]>")]
        );
        assert_eq!(
            document
                .nodes()
                .find_map(Node::as_cdata_section)
                .map(|section| section.data()),
            Some("real")
        );
    }

    #[test]
    fn incremental_xml_does_not_suspend_for_non_executable_script_namespace() {
        let mut stream = XmlDocumentStream::new(test_url());
        stream.append_to_end("<root><script>not executable</script><tail/></root>".to_owned());
        stream.declare_eof();

        let outcome = stream.pump_next_parser_step(0);
        assert!(matches!(outcome.result, ParserPumpStep::InputDrained));
        assert_eq!(
            stream
                .snapshot_parser_stream_document()
                .elements_by_tag_name(
                    stream.snapshot_parser_stream_document().document_node_id(),
                    "tail",
                    false,
                )
                .len(),
            1
        );
    }

    #[test]
    fn incremental_top_level_unstyled_xml_builds_viewer_on_the_same_source_nodes() {
        let mut stream = XmlDocumentStream::new_top_level_document(test_url());
        stream.append_to_end(
            "<semantic-root id='source'><child>ready</child></semantic-root>".to_owned(),
        );
        stream.declare_eof();
        while stream.has_pending_input() {
            let outcome = stream.pump_next_parser_step(2);
            assert!(matches!(outcome.result, ParserPumpStep::InputDrained));
        }
        let source_root = stream
            .snapshot_parser_stream_dom_host()
            .element_handle_by_id("source")
            .expect("source XML root before viewer conversion");

        let document = stream.finish();
        let document_element = document
            .document_element_handle()
            .and_then(|handle| document.node(handle))
            .and_then(Node::as_element)
            .expect("viewer document element");
        assert_eq!(document_element.local_name(), "html");
        assert_eq!(document_element.namespace(), "http://www.w3.org/1999/xhtml");
        let source_container = element_by_id(&document, "webkit-xml-viewer-source-xml")
            .expect("viewer source container");
        assert_eq!(
            document
                .child_ids(source_container)
                .find(|handle| document.node(*handle).is_some_and(Node::is_element)),
            Some(source_root),
            "viewer conversion must move rather than clone the parsed XML source root"
        );
    }

    #[test]
    fn incremental_top_level_xhtml_keeps_its_xml_tree() {
        let mut stream = XmlDocumentStream::new_top_level_document(test_url());
        stream.append_to_end(
            "<html xmlns='http://www.w3.org/1999/xhtml'><body id='body'/></html>".to_owned(),
        );
        let document = stream.finish();
        assert!(element_by_id(&document, "body").is_some());
        assert!(element_by_id(&document, "webkit-xml-viewer-source-xml").is_none());
    }

    #[test]
    fn incremental_top_level_malformed_xml_does_not_build_viewer() {
        let mut stream = XmlDocumentStream::new_top_level_document(test_url());
        stream.append_to_end("<semantic-root>".to_owned());
        let document = stream.finish();
        assert!(!document.parse_errors().is_empty());
        assert_eq!(
            document
                .document_element_handle()
                .and_then(|handle| document.node(handle))
                .and_then(Node::as_element)
                .map(|element| element.local_name()),
            Some("semantic-root")
        );
        assert!(element_by_id(&document, "webkit-xml-viewer-source-xml").is_none());
    }
}
