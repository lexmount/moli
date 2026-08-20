use std::rc::Rc;

use html5ever::{
    LocalName, Namespace, ParseOpts, QualName,
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
    },
    tree_builder::{TreeBuilder, TreeBuilderOpts, TreeSink},
};
use markup5ever::TokenizerResult;
use moli_dom::native::NativeNodeId;

use super::{
    html::{DocumentSink, ParseHandle, ParserFinishDiscoverySignals, ParserInputQueue},
    html_input::InputStack,
    live_target::ParserStreamHtmlTreeSinkTarget,
};

pub(super) struct HtmlTreeSinkSession {
    pub(super) parser: HtmlParserSession,
    pub(super) script_input: ParserInputQueue,
}

pub(super) struct HtmlParserSession {
    tokenizer: Tokenizer<EmbedderPausingTreeBuilder>,
    input: InputStack,
}

pub(super) enum HtmlParserSessionResult {
    InputDrained,
    Script(ParseHandle),
}

struct EmbedderPausingTreeBuilder {
    inner: TreeBuilder<ParseHandle, DocumentSink>,
}

impl EmbedderPausingTreeBuilder {
    fn new(sink: DocumentSink, opts: TreeBuilderOpts) -> Self {
        Self {
            inner: TreeBuilder::new(sink, opts),
        }
    }

    fn new_for_fragment(
        sink: DocumentSink,
        context_handle: ParseHandle,
        opts: TreeBuilderOpts,
    ) -> Self {
        Self {
            inner: TreeBuilder::new_for_fragment(sink, context_handle, None, opts),
        }
    }

    fn sink(&self) -> &DocumentSink {
        &self.inner.sink
    }
}

impl TokenSink for EmbedderPausingTreeBuilder {
    type Handle = ParseHandle;

    fn process_token(&self, token: Token, line_number: u64) -> TokenSinkResult<Self::Handle> {
        let in_foreign_content = self
            .inner
            .adjusted_current_node_present_but_not_in_html_namespace();
        let foreign_end_tag = match &token {
            Token::TagToken(tag) if in_foreign_content && tag.kind == TagKind::EndTag => {
                Some(tag.name.clone())
            }
            _ => None,
        };
        let self_closing_foreign_start_tag = match &token {
            Token::TagToken(tag)
                if in_foreign_content && tag.kind == TagKind::StartTag && tag.self_closing =>
            {
                Some(tag.name.clone())
            }
            _ => None,
        };
        let result = self.inner.process_token(token, line_number);
        if !matches!(result, TokenSinkResult::Continue) {
            return result;
        }
        let svg_script_handoff = if let Some(local_name) = foreign_end_tag {
            // html5ever truncates its foreign-content open-element stack
            // directly, bypassing TreeSink::pop(). Keep the parser target's
            // mirror synchronized and expose closed SVG scripts below.
            self.inner
                .sink
                .note_foreign_end_tag_processed(local_name.as_ref())
        } else if let Some(local_name) = self_closing_foreign_start_tag {
            self.inner
                .sink
                .note_self_closing_foreign_element_processed(local_name.as_ref())
        } else {
            None
        };
        if let Some(script) = svg_script_handoff {
            // html5ever 0.39 has an explicit FIXME for </script> in SVG and
            // does not return TokenSinkResult::Script for it. Preserve the
            // ordinary tokenizer pause contract at this narrow adapter.
            return TokenSinkResult::Script(ParseHandle::new(script, None));
        }
        if let Some(placeholder) = self
            .inner
            .sink
            .pending_custom_element_construction_handoff_placeholder()
        {
            // html5ever has no custom-element pause result. Moli interprets the
            // script handoff handle as a custom-element handoff when the parser
            // sink has a matching pending construction record.
            return TokenSinkResult::Script(ParseHandle::new(placeholder, None));
        }
        if let Some(stylesheet) = self.inner.sink.pending_blocking_stylesheet_pause() {
            // html5ever exposes one generic tokenizer-yield result.  The stream
            // layer distinguishes this parser-created stylesheet boundary from
            // actual script and custom-element handoffs using sink-owned state.
            return TokenSinkResult::Script(ParseHandle::new(stylesheet, None));
        }
        TokenSinkResult::Continue
    }

    fn end(&self) {
        self.inner.end()
    }

    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.inner
            .adjusted_current_node_present_but_not_in_html_namespace()
    }
}

impl HtmlParserSession {
    fn new(sink: DocumentSink, opts: ParseOpts) -> Self {
        let tree_builder = EmbedderPausingTreeBuilder::new(sink, opts.tree_builder);
        Self {
            tokenizer: Tokenizer::new(tree_builder, opts.tokenizer),
            input: InputStack::default(),
        }
    }

    pub(super) fn new_fragment(
        sink: DocumentSink,
        opts: ParseOpts,
        context_handle: ParseHandle,
        context_element_allows_scripting: bool,
    ) -> Self {
        let tree_builder =
            EmbedderPausingTreeBuilder::new_for_fragment(sink, context_handle, opts.tree_builder);
        let tokenizer_options = TokenizerOpts {
            initial_state: Some(
                tree_builder
                    .inner
                    .tokenizer_state_for_context_elem(context_element_allows_scripting),
            ),
            ..opts.tokenizer
        };
        Self {
            tokenizer: Tokenizer::new(tree_builder, tokenizer_options),
            input: InputStack::default(),
        }
    }

    pub(super) fn process(&mut self, input: StrTendril) {
        self.input.push_back(input);
        while let HtmlParserSessionResult::Script(_) =
            feed_with_definitive_encoding(&self.tokenizer, self.input.current())
        {
            // Non-pump callers intentionally parse through embedder pauses. They have no
            // runtime owner to notify, so parser-side custom-element handoffs and
            // blocking-stylesheet pauses must be discarded before continuing.
            self.discard_parser_side_embedder_yield();
        }
    }

    pub(super) fn push_back(&mut self, input: StrTendril) {
        self.input.push_back(input);
    }

    pub(super) fn begin_inserted_input(&mut self, input: StrTendril) {
        if input.is_empty() {
            return;
        }
        // Without segment provenance from html5ever, any later token can span
        // inserted input and the original tail after this insertion frame is
        // restored. Prefer permanent unknown locations over reporting
        // plausible but incorrect document lines.
        self.tokenizer.sink.sink().mark_source_positions_unknown();
        self.input.begin_inserted(input);
    }

    pub(super) fn append_to_current_inserted_input(&mut self, input: StrTendril) -> bool {
        self.input.append_to_current_inserted(input)
    }

    pub(super) fn has_buffered_input(&self) -> bool {
        self.input.has_input()
    }

    pub(super) fn buffered_input_len(&self) -> usize {
        self.input.len()
    }

    pub(super) fn snapshot_buffered_input(&self) -> String {
        self.input.snapshot()
    }

    pub(super) fn feed(&mut self) -> HtmlParserSessionResult {
        let result = feed_with_definitive_encoding(&self.tokenizer, self.input.current());
        if matches!(result, HtmlParserSessionResult::InputDrained) {
            // The restored parent is intentionally consumed by the next parser
            // step so each insertion depth keeps an explicit input boundary.
            self.input.restore_parent_if_current_empty();
        }
        result
    }

    pub(super) fn sink(&self) -> &DocumentSink {
        self.tokenizer.sink.sink()
    }

    pub(super) fn finish(self) -> ParserStreamHtmlTreeSinkTarget {
        let Self { tokenizer, input } = self;
        let input_buffer = input.into_buffer();
        while let HtmlParserSessionResult::Script(_) =
            feed_with_definitive_encoding(&tokenizer, &input_buffer)
        {
            tokenizer
                .sink
                .sink()
                .pop_pending_custom_element_construction_handoff();
            tokenizer
                .sink
                .sink()
                .pop_pending_blocking_stylesheet_pause();
        }
        debug_assert!(input_buffer.is_empty());
        tokenizer.sink.sink().begin_tree_builder_finish();
        tokenizer.end();
        tokenizer.sink.inner.sink.finish()
    }

    pub(super) fn finish_live_runtime_dom_sink_parser(self) -> ParserFinishDiscoverySignals {
        let Self { tokenizer, input } = self;
        let input_buffer = input.into_buffer();
        while let HtmlParserSessionResult::Script(_) =
            feed_with_definitive_encoding(&tokenizer, &input_buffer)
        {
            tokenizer
                .sink
                .sink()
                .pop_pending_custom_element_construction_handoff();
            tokenizer
                .sink
                .sink()
                .pop_pending_blocking_stylesheet_pause();
        }
        debug_assert!(input_buffer.is_empty());
        tokenizer.sink.sink().begin_tree_builder_finish();
        tokenizer.end();
        let sink = tokenizer.sink.sink();
        ParserFinishDiscoverySignals {
            parser_created_null_registry_elements: sink
                .take_parser_stream_null_custom_element_registry_elements(),
            discovered_modulepreload_link_candidates: sink
                .drain_discovered_modulepreload_link_candidates(),
            discovered_parser_meta_csp_candidates: sink
                .drain_discovered_parser_meta_csp_candidates(),
            discovered_blocking_stylesheet_inputs: sink
                .drain_discovered_blocking_stylesheet_inputs(),
        }
    }

    fn discard_parser_side_embedder_yield(&self) {
        self.tokenizer
            .sink
            .sink()
            .pop_pending_custom_element_construction_handoff();
        self.tokenizer
            .sink
            .sink()
            .pop_pending_blocking_stylesheet_pause();
    }
}

fn feed_with_definitive_encoding(
    tokenizer: &Tokenizer<EmbedderPausingTreeBuilder>,
    input_buffer: &BufferQueue,
) -> HtmlParserSessionResult {
    loop {
        match tokenizer.feed(input_buffer) {
            // Moli resolves the document encoding from the response and byte-level
            // meta prescan before creating this Unicode parser session. The tree
            // builder cannot change that definitive decoding, so continue past its
            // advisory notification without exposing a false parser pause.
            TokenizerResult::EncodingIndicator(_) => {}
            TokenizerResult::Done => return HtmlParserSessionResult::InputDrained,
            TokenizerResult::Script(handle) => {
                return HtmlParserSessionResult::Script(handle);
            }
        }
    }
}

pub(super) fn new_html_tree_sink_session(
    target: ParserStreamHtmlTreeSinkTarget,
) -> HtmlTreeSinkSession {
    let sink = DocumentSink::new(target);
    let parser = HtmlParserSession::new(sink, html_parse_opts());
    let script_input = ParserInputQueue::default();

    HtmlTreeSinkSession {
        parser,
        script_input,
    }
}

pub(super) fn new_fragment_html_tree_sink_session(
    target: ParserStreamHtmlTreeSinkTarget,
    context_handle: NativeNodeId,
    context_namespace: &str,
    context_local_name: &str,
) -> HtmlTreeSinkSession {
    let sink = DocumentSink::new(target);
    let context = QualName::new(
        None,
        Namespace::from(context_namespace),
        LocalName::from(context_local_name),
    );
    let context_handle = ParseHandle::new(context_handle, Some(Rc::new(context)));
    let parser = HtmlParserSession::new_fragment(sink, html_parse_opts(), context_handle, true);
    let script_input = ParserInputQueue::default();

    HtmlTreeSinkSession {
        parser,
        script_input,
    }
}

pub(super) fn html_parse_opts() -> ParseOpts {
    html_parse_opts_with_scripting(true)
}

pub(super) fn html_parse_opts_with_scripting(scripting_enabled: bool) -> ParseOpts {
    ParseOpts {
        tree_builder: TreeBuilderOpts {
            scripting_enabled,
            ..TreeBuilderOpts::default()
        },
        ..ParseOpts::default()
    }
}
